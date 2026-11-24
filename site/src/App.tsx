import Nav from './components/Nav'
import ProductBar from './components/ProductBar'
import Hero from './sections/Hero'
import Highlights from './sections/Highlights'
import Agents from './sections/Agents'
import Ssh from './sections/Ssh'
import FeatureChapters from './sections/FeatureChapters'
import Platforms from './sections/Platforms'
import Compare from './sections/Compare'
import Features from './sections/Features'
import Shortcuts from './sections/Shortcuts'
import Install from './sections/Install'
import Faq from './sections/Faq'
import Footer from './sections/Footer'

/** Laid out like a phone's product page: an opener, one chapter per big feature, specs, then "Get nex." */
export default function App() {
  return (
    <>
      <Nav />
      <ProductBar />
      <main>
        <Hero />
        <Highlights />
        <Agents />
        <Ssh />
        <FeatureChapters />
        <Platforms />
        <Compare />
        <Features />
        <Shortcuts />
        <Faq />
        <Install />
      </main>
      <Footer />
    </>
  )
}
