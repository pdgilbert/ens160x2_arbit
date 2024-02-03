#![deny(unsafe_code)]
#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

#[cfg(debug_assertions)]
use panic_semihosting as _;

#[cfg(not(debug_assertions))]
use panic_halt as _;

use rtic::app;

#[cfg_attr(feature = "stm32f4xx", app(device = stm32f4xx_hal::pac,   dispatchers = [TIM2, TIM3]))]
#[cfg_attr(feature = "stm32h7xx", app(device = stm32h7xx_hal::pac,   dispatchers = [TIM2, TIM3]))]

mod app {

   use rtic;
   use rtic_monotonics::systick::Systick;
   use rtic_monotonics::systick::fugit::{ExtU32};

   use cortex_m_semihosting::{hprintln};

   const MONOCLOCK: u32 = 8_000_000; 

   const READ_INTERVAL: u32 = 10;  // used as seconds

  
   /////////////////////   arbiter
   use core::mem::MaybeUninit;
   use rtic_sync::{arbiter::{i2c::ArbiterDevice, Arbiter}};


   /////////////////////   ens
   use ens160::{Ens160, AirQualityIndex, ECo2};


   /////////////////////   hals
   
   use embedded_hal::{
      i2c::I2c as I2cTrait,
      delay::DelayNs,
   };
   
   #[cfg(feature = "stm32f4xx")]            
   use stm32f4xx_hal as hal;

   #[cfg(feature = "stm32h7xx")]
   use stm32h7xx_hal as hal;

   use hal::{
      pac::{Peripherals, I2C1}, 
      i2c::I2c,  //as I2cType,
      rcc::{RccExt},
      prelude::*,
   };
   
   #[cfg(feature = "stm32h7xx")]
   use stm32h7xx_hal::{
      delay::DelayFromCountDownTimer,
   };
   
   /////////////////////   setup
 
   #[cfg(feature = "stm32f4xx")]            
   pub fn setup_from_dp(dp: Peripherals) ->  ( I2c<I2C1>, impl DelayNs) { 
      let rcc = dp.RCC.constrain();
      let clocks = rcc.cfgr.freeze();
   
      let gpiob = dp.GPIOB.split();
      let scl = gpiob.pb8.into_alternate_open_drain(); 
      let sda = gpiob.pb9.into_alternate_open_drain(); 
   
      let i2c = dp.I2C1.i2c( (scl, sda), 400.kHz(), &clocks);
   
      let delay = dp.TIM5.delay::<1000000_u32>(&clocks);

      (i2c, delay)
   }

   #[cfg(feature = "stm32h7xx")]
   pub fn setup_from_dp(dp: Peripherals) ->  ( I2c<I2C1>, impl DelayNs) { 
      let rcc = dp.RCC.constrain();
      let vos = dp.PWR.constrain().freeze();
      let ccdr = rcc.sys_ck(100.MHz()).freeze(vos, &dp.SYSCFG); 
      let clocks = ccdr.clocks;
   
      let gpiob = dp.GPIOB.split(ccdr.peripheral.GPIOB);
      let scl = gpiob.pb8.into_alternate().set_open_drain();
      let sda = gpiob.pb9.into_alternate().set_open_drain();
   
      let i2c = dp.I2C1.i2c((scl, sda), 400.kHz(), ccdr.peripheral.I2C1, &clocks);
   
      // CountDownTimer not supported by embedded-hal 1.0.0
      let timer = dp.TIM5.timer(1.Hz(), ccdr.peripheral.TIM5, &clocks);
      let delay = DelayFromCountDownTimer::new(timer);
   
      (i2c, delay)
   }
   
////////////////////////////////////////////////////////////////////////////////
    #[shared]
    struct Shared {}

    #[local]
    struct Local {
        ens_1: Ens160<ArbiterDevice<'static, I2c<'static, I2C1>>>,
        ens_2: Ens160<ArbiterDevice<'static, I2c<'static, I2C1>>>,
    }

////////////////////////////////////////////////////////////////////////////////

    #[init(local = [
        i2c_arbiter: MaybeUninit<Arbiter<I2c<'static, I2C1>>> = MaybeUninit::uninit(),
    ])]
    fn init(cx: init::Context) -> (Shared, Local ) {

       let mono_token = rtic_monotonics::create_systick_token!();
       Systick::start(cx.core.SYST, MONOCLOCK, mono_token);

       let (i2c, _delay) = setup_from_dp(cx.device);

       let i2c_arbiter = cx.local.i2c_arbiter.write(Arbiter::new(i2c));

       let ens_1 = Ens160::new(ArbiterDevice::new(i2c_arbiter), 0x52);
       let ens_2 = Ens160::new(ArbiterDevice::new(i2c_arbiter), 0x53);

       i2c_sensors::spawn(i2c_arbiter).ok();

       (Shared {}, Local { ens_1, ens_2 })
    }

    #[task(local = [ens_1, ens_2])]
    async fn i2c_sensors(cx: i2c_sensors::Context, i2c: &'static Arbiter<I2c<'static, I2C1>>) {
        
        let mut tvoc_1: u16;
        let mut eco_1: ECo2;
        let mut aqia_1: AirQualityIndex ;
        let mut aqib_1: AirQualityIndex ;
        
        let mut tvoc_2: u16;
        let mut eco_2: ECo2;
        let mut aqia_2: AirQualityIndex ;
        let mut aqib_2: AirQualityIndex ;

        let ens_1 = cx.local.ens_1;
        let ens_2 = cx.local.ens_2;

        loop {
//            {
//                // Use scope to make sure I2C access is dropped.
//                // Read from sensor driver that wants to use I2C directly.
//             ?? read data before reading status ??
//                let mut i2c = i2c.access().await;
//                let status = Asensor::status(&mut i2c).await;
//            }

            if let Ok(status) = ens_1.status() {
                if status.data_is_ready() {
                    tvoc_1 = ens_1.tvoc().await;
                    eco_1  = ens_1.eco().await;
                    aqia_1 = AirQualityIndex::try_from(eco_1).unwrap(); // from eco
                    aqib_1 = ens_1.air_quality_index().await;         // directly
                }
            }

            if let Ok(status) = ens_2.status() {
                 if status.data_is_ready() {
                     tvoc_1 = ens_2.tvoc().await;
                     eco_1  = ens_2.eco2().await;
                     aqia_1 = AirQualityIndex::try_from(eco_2).unwrap(); // from eco
                     aqia_1 = ens_2.air_quality_index().await;         // directly
                 }
             }
    
        hprintln!("S1: TVOC:{} ppb. eco:{:?} AQIa: {:?}  b:{:?}", tvoc_1, eco_1, aqia_1, aqib_1).unwrap();
        Systick::delay(5.secs()).await;
        hprintln!("S2: TVOC:{} ppb. eco:{:?} AQIa: {:?}  b:{:?}", tvoc_2, eco_2, aqia_2, aqib_2).unwrap();

        Systick::delay(5.secs()).await;
        }
    }
}
     
     
     
     

     
     
     

     
     
     
     
   

     
     
     
     
     
     
     

     
